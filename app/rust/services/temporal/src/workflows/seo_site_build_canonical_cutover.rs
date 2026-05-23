use contracts::generated::alegria::temporal::v1::{
    CmsApprovalDecision, CmsPublishInputPayload, CmsPublishOutputPayload,
    ContentContractValidateInputPayload, CrawlSourcesInputPayload, DraftAssembleInputPayload,
    DraftAssembleOutputPayload, DraftNormalizeInputPayload, DraftNormalizeOutputPayload,
    DraftQaInputPayload, EditorialDraftGenerateInputPayload, EditorialDraftGenerateOutputPayload,
    FinalizePublishInputPayload, GlobalSiteReconcileInputPayload, IaBuildInputPayload,
    LinkRecommendInputPayload, OpportunityBuildInputPayload, ProjectionBarrierAuditInputPayload,
    PublishMaterializeInputPayload, PublishMaterializeOutputPayload,
    RebuildDetectInputPayload, RenderPreviewValidateInputPayload, SeoSiteBuildInputPayload,
    SeoVerifiedFactSupportState, SerpIngestInputPayload, SerpNormalizeInputPayload,
};
use infrastructure::adapters::temporalio_sdk_adapter::{
    workflow, workflow_methods, SyncWorkflowContext, WorkerOptions, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};
use seo_application::execution::{
    blocked_interaction_status, blocked_publish_gate_status, build_execution_plan,
    final_phase_keys, normalize_run_mode, page_phase_keys, phase_label, policy_for_run_mode,
    scenario_kind_for_run_mode, RunInteractionPolicy, SeoPhaseKey,
};
use seo_application::scenario::{SeoExecutionMode, SeoScenarioRequest};
use seo_ports::VerifiedSupportBundleRequest;

use crate::activities::operations::{
    CandidateValidationInput, CanonicalMappingSweepInput, CasGateInput,
    CommercialSignalExtractionSweepInput, CompletenessJudgeSweepInput,
    ContradictionGateSweepInput, DomBlockRelevanceSweepInput,
    EditorialExtractionSweepInput, EntitySpanSweepInput, ExtractionSchemaValidateInput,
    HumanApprovalWaitInput,
    LayerRouterSweepInput, OntologyIntakeGateInput, OperationalExtractionSweepInput,
    PageUtilitySweepInput, ProceduralExtractionSweepInput, ProjectionSyncInput,
    ProjectionSyncOutput, RawEvidenceRegisterInput, ResolutionLoopInput, SectionSemanticGateBundle, SectioningContractGateInput,
    SectioningInput, SeoPreflightInput, SeoSignalExtractionSweepInput,
    SubspanLayerRouterInput, TripleBuilderSweepInput, TruthAdjudicationSweepInput,
    TruthAdmissibilityGateInput, VerifiedTruthWriteInput, WholePageSemanticPassInput,
};
use crate::activities::AlegriaActivities;
use crate::metrics;

use super::runtime::{db_opts, BasicWorkflowStatus};

#[workflow]
#[derive(Default)]
struct SeoSiteBuildCanonicalCutoverWorkflow {
    paused: bool,
    waiting_hitl: bool,
    resume_requested: bool,
    phase: String,
}

pub(crate) fn register(opts: &mut WorkerOptions) {
    opts.register_workflow::<SeoSiteBuildCanonicalCutoverWorkflow>();
}

fn support_request(run_id: &str, site_input: &SeoSiteBuildInputPayload) -> WorkflowResult<String> {
    let scope = site_input
        .scope
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("SeoSiteBuildInputPayload.scope is required"))?;
    Ok(serde_json::to_string(&VerifiedSupportBundleRequest {
        run_id: run_id.to_string(),
        context_key: site_input.context_key.clone(),
        scope_signature: scope.scope_signature.clone(),
        applicant_profile: scope.applicant_profile.clone(),
    })
    .map_err(anyhow::Error::from)?)
}

fn phase_with_ordinal(key: SeoPhaseKey, page_index: usize, page_total: usize) -> String {
    format!("{}:{}/{}", phase_label(key), page_index + 1, page_total)
}

async fn sync_target(
    ctx: &mut WorkflowContext<SeoSiteBuildCanonicalCutoverWorkflow>,
    run_id: &str,
    step_name: &str,
    target_system: &str,
) -> WorkflowResult<ProjectionSyncOutput> {
    Ok(ctx
        .start_activity(
            AlegriaActivities::run_projection_sync_step,
            ProjectionSyncInput {
                run_id: run_id.to_string(),
                step_name: step_name.to_string(),
                target_system: target_system.to_string(),
                batch_limit: 500,
                lease_seconds: 120,
            },
            db_opts(120),
        )
        .await?)
}

#[workflow_methods]
impl SeoSiteBuildCanonicalCutoverWorkflow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        metrics::global()
            .workflow_starts_total
            .with_label_values(&["SeoSiteBuildCanonicalCutoverWorkflow"])
            .inc();

        let run_id = ctx.workflow_initial_info().workflow_id.clone();
        ctx.state_mut(|s| s.phase = "load_seo_site_build_input".to_string());
        let site_input: SeoSiteBuildInputPayload = ctx
            .start_activity(
                AlegriaActivities::load_seo_site_build_input,
                run_id.clone(),
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;
        let scope = site_input
            .scope
            .clone()
            .ok_or_else(|| anyhow::anyhow!("SeoSiteBuildInputPayload.scope is required"))?;
        let normalized_run_mode = normalize_run_mode(&site_input.run_mode);
        let request = SeoScenarioRequest {
            scenario: scenario_kind_for_run_mode(normalized_run_mode),
            mode: SeoExecutionMode::TemporalDurable,
            policy: policy_for_run_mode(normalized_run_mode, SeoExecutionMode::TemporalDurable),
            output_dir: String::new(),
            base_url: String::new(),
            site_input: site_input.clone(),
        };
        let plan = build_execution_plan(&request);

        ctx.state_mut(|s| s.phase = "seo_preflight".to_string());
        let preflight = ctx
            .start_activity(
                AlegriaActivities::run_seo_preflight_step,
                SeoPreflightInput {
                    run_id: run_id.clone(),
                    context_key: site_input.context_key.clone(),
                    scope: scope.clone(),
                    projection_max_lag_ms: 300_000,
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        let support_bundle_request = support_request(&run_id, &site_input)?;
        ctx.state_mut(|s| s.phase = "load_verified_support_bundle.initial".to_string());
        let mut support_bundle: Vec<SeoVerifiedFactSupportState> = ctx
            .start_activity(
                AlegriaActivities::load_verified_support_bundle,
                support_bundle_request.clone(),
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "serp_ingest".to_string());
        let ingest = ctx
            .start_activity(
                AlegriaActivities::run_serp_ingest_step,
                SerpIngestInputPayload {
                    run_id: run_id.clone(),
                    query_batch_key: site_input.query_batch_key.clone(),
                    scope: Some(scope.clone()),
                    queries: site_input.queries.clone(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "crawl_sources".to_string());
        let crawled = ctx
            .start_activity(
                AlegriaActivities::run_crawl_sources_step,
                CrawlSourcesInputPayload {
                    run_id: run_id.clone(),
                    query_batch_key: ingest.query_batch_key.clone(),
                    limit: 25,
                    emit_qdrant: true,
                },
                db_opts(120),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "whole_page_semantic_pass".to_string());
        let semantic = ctx
            .start_activity(
                AlegriaActivities::run_whole_page_semantic_pass_step,
                WholePageSemanticPassInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "page_utility_classifier".to_string());
        let page_utility = ctx
            .start_activity(
                AlegriaActivities::run_page_utility_sweep_step,
                PageUtilitySweepInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "dom_block_relevance_filter".to_string());
        let dom = ctx
            .start_activity(
                AlegriaActivities::run_dom_block_relevance_sweep_step,
                DomBlockRelevanceSweepInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "sectioning".to_string());
        let sectioning = ctx
            .start_activity(
                AlegriaActivities::run_sectioning_step,
                SectioningInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "sectioning_contract_gate".to_string());
        let sectioning_contract = ctx
            .start_activity(
                AlegriaActivities::run_sectioning_contract_gate_step,
                SectioningContractGateInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "cas_gate".to_string());
        let cas_gate = ctx
            .start_activity(
                AlegriaActivities::run_cas_gate_step,
                CasGateInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "raw_evidence_register".to_string());
        let raw_evidence = ctx
            .start_activity(
                AlegriaActivities::run_raw_evidence_register_step,
                RawEvidenceRegisterInput {
                    run_id: run_id.clone(),
                    context_key: site_input.context_key.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "projection_barrier(raw_evidence)".to_string());
        let raw_evidence_barrier = ctx
            .start_activity(
                AlegriaActivities::run_projection_barrier_audit_step,
                ProjectionBarrierAuditInputPayload {
                    run_id: run_id.clone(),
                    checkpoint: "projection_barrier(raw_evidence)".to_string(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        let semantic_gates = SectionSemanticGateBundle {
            page_utility: page_utility.clone(),
            dom_relevance: dom.clone(),
            sectioning_contract: sectioning_contract.clone(),
            cas_gate: cas_gate.clone(),
        };

        ctx.state_mut(|s| s.phase = "layer_router".to_string());
        let layer_router = ctx
            .start_activity(
                AlegriaActivities::run_layer_router_sweep_step,
                LayerRouterSweepInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                    gates: semantic_gates.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "subspan_layer_router".to_string());
        let subspan_layer_router = ctx
            .start_activity(
                AlegriaActivities::run_subspan_layer_router_step,
                SubspanLayerRouterInput {
                    run_id: run_id.clone(),
                    layer_router: layer_router.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "entity_span_detection".to_string());
        let entity_spans = ctx
            .start_activity(
                AlegriaActivities::run_entity_span_sweep_step,
                EntitySpanSweepInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                    gates: semantic_gates,
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "canonical_mapping".to_string());
        let canonical_mapping = ctx
            .start_activity(
                AlegriaActivities::run_canonical_mapping_sweep_step,
                CanonicalMappingSweepInput {
                    run_id: run_id.clone(),
                    entity_spans: entity_spans.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "ontology_intake_gate".to_string());
        let ontology_intake = ctx
            .start_activity(
                AlegriaActivities::run_ontology_intake_gate_step,
                OntologyIntakeGateInput {
                    run_id: run_id.clone(),
                    canonical_mapping: canonical_mapping.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "procedural_extraction".to_string());
        let procedural = ctx
            .start_activity(
                AlegriaActivities::run_procedural_extraction_sweep_step,
                ProceduralExtractionSweepInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                    gates: SectionSemanticGateBundle {
                        page_utility: page_utility.clone(),
                        dom_relevance: dom.clone(),
                        sectioning_contract: sectioning_contract.clone(),
                        cas_gate: cas_gate.clone(),
                    },
                    ontology: ontology_intake.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "operational_extraction".to_string());
        let operational = ctx
            .start_activity(
                AlegriaActivities::run_operational_extraction_sweep_step,
                OperationalExtractionSweepInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                    gates: SectionSemanticGateBundle {
                        page_utility: page_utility.clone(),
                        dom_relevance: dom.clone(),
                        sectioning_contract: sectioning_contract.clone(),
                        cas_gate: cas_gate.clone(),
                    },
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "editorial_extraction".to_string());
        let editorial = ctx
            .start_activity(
                AlegriaActivities::run_editorial_extraction_sweep_step,
                EditorialExtractionSweepInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                    gates: SectionSemanticGateBundle {
                        page_utility: page_utility.clone(),
                        dom_relevance: dom.clone(),
                        sectioning_contract: sectioning_contract.clone(),
                        cas_gate: cas_gate.clone(),
                    },
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "seo_signal_extraction".to_string());
        let seo_signals = ctx
            .start_activity(
                AlegriaActivities::run_seo_signal_extraction_step,
                SeoSignalExtractionSweepInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                    gates: SectionSemanticGateBundle {
                        page_utility: page_utility.clone(),
                        dom_relevance: dom.clone(),
                        sectioning_contract: sectioning_contract.clone(),
                        cas_gate: cas_gate.clone(),
                    },
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "commercial_signal_extraction".to_string());
        let commercial_signals = ctx
            .start_activity(
                AlegriaActivities::run_commercial_signal_extraction_step,
                CommercialSignalExtractionSweepInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                    gates: SectionSemanticGateBundle {
                        page_utility: page_utility.clone(),
                        dom_relevance: dom.clone(),
                        sectioning_contract: sectioning_contract.clone(),
                        cas_gate: cas_gate.clone(),
                    },
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "extraction_schema_validate".to_string());
        let schema_validate = ctx
            .start_activity(
                AlegriaActivities::run_extraction_schema_validate_step,
                ExtractionSchemaValidateInput {
                    run_id: run_id.clone(),
                    procedural: procedural.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "candidate_validation".to_string());
        let candidate_validation = ctx
            .start_activity(
                AlegriaActivities::run_candidate_validation_step,
                CandidateValidationInput {
                    run_id: run_id.clone(),
                    context_key: site_input.context_key.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                    procedural: procedural.clone(),
                    ontology: ontology_intake.clone(),
                    schema_validate: schema_validate.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "triple_builder".to_string());
        let triples = ctx
            .start_activity(
                AlegriaActivities::run_triple_builder_sweep_step,
                TripleBuilderSweepInput {
                    run_id: run_id.clone(),
                    procedural: procedural.clone(),
                    operational: operational.clone(),
                    editorial: editorial.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "completeness_judge".to_string());
        let completeness = ctx
            .start_activity(
                AlegriaActivities::run_completeness_judge_sweep_step,
                CompletenessJudgeSweepInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                    entity_spans: entity_spans.clone(),
                    procedural: procedural.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "resolution_loop".to_string());
        let resolution = ctx
            .start_activity(
                AlegriaActivities::run_resolution_loop_step,
                ResolutionLoopInput {
                    run_id: run_id.clone(),
                    ontology: ontology_intake.clone(),
                    schema_validate: schema_validate.clone(),
                    completeness: completeness.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "contradiction_gate".to_string());
        let contradiction = ctx
            .start_activity(
                AlegriaActivities::run_contradiction_gate_sweep_step,
                ContradictionGateSweepInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                    procedural: procedural.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "truth_adjudication".to_string());
        let truth_adjudication = ctx
            .start_activity(
                AlegriaActivities::run_truth_adjudication_step,
                TruthAdjudicationSweepInput {
                    run_id: run_id.clone(),
                    context_key: site_input.context_key.clone(),
                    candidate_validation: candidate_validation.clone(),
                    resolution: resolution.clone(),
                    contradiction: contradiction.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "verified_truth_write".to_string());
        let verified_truth_write = ctx
            .start_activity(
                AlegriaActivities::run_verified_truth_write_step,
                VerifiedTruthWriteInput {
                    run_id: run_id.clone(),
                    context_key: site_input.context_key.clone(),
                    truth_adjudication: truth_adjudication.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        if !verified_truth_write.changed_truth_keys.is_empty() {
            ctx.state_mut(|s| s.phase = "load_verified_support_bundle.refresh".to_string());
            support_bundle = ctx
                .start_activity(
                    AlegriaActivities::load_verified_support_bundle,
                    support_bundle_request,
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;
        }

        ctx.state_mut(|s| s.phase = "graph_admissibility_gate".to_string());
        let graph_gate = ctx
            .start_activity(
                AlegriaActivities::run_projection_barrier_audit_step,
                ProjectionBarrierAuditInputPayload {
                    run_id: run_id.clone(),
                    checkpoint: "graph_admissibility_gate".to_string(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "retrieval_admissibility_gate".to_string());
        let retrieval_gate = ctx
            .start_activity(
                AlegriaActivities::run_projection_barrier_audit_step,
                ProjectionBarrierAuditInputPayload {
                    run_id: run_id.clone(),
                    checkpoint: "retrieval_admissibility_gate".to_string(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "neo4j_sync".to_string());
        let neo4j = sync_target(ctx, &run_id, "neo4j_sync", "neo4j").await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "voyage_qdrant_sync".to_string());
        let qdrant = sync_target(ctx, &run_id, "voyage_qdrant_sync", "qdrant").await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "projection_barrier(semantic_projection)".to_string());
        let semantic_projection_barrier = ctx
            .start_activity(
                AlegriaActivities::run_projection_barrier_audit_step,
                ProjectionBarrierAuditInputPayload {
                    run_id: run_id.clone(),
                    checkpoint: "projection_barrier(semantic_projection)".to_string(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "serp_normalize".to_string());
        let serp_normalize = ctx
            .start_activity(
                AlegriaActivities::run_serp_normalize_step,
                SerpNormalizeInputPayload {
                    run_id: run_id.clone(),
                    query_batch_key: ingest.query_batch_key.clone(),
                    scope: Some(scope.clone()),
                    queries: site_input.queries.clone(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "opportunity_build".to_string());
        let opportunity_build = ctx
            .start_activity(
                AlegriaActivities::run_opportunity_build_step,
                OpportunityBuildInputPayload {
                    run_id: run_id.clone(),
                    scope: Some(scope.clone()),
                    serp_patterns: serp_normalize.serp_patterns.clone(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "ia_build".to_string());
        let ia_build = ctx
            .start_activity(
                AlegriaActivities::run_ia_build_step,
                IaBuildInputPayload {
                    run_id: run_id.clone(),
                    scope: Some(scope.clone()),
                    keyword_clusters: opportunity_build.keyword_clusters.clone(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "link_recommend".to_string());
        let link_recommend = ctx
            .start_activity(
                AlegriaActivities::run_link_recommend_step,
                LinkRecommendInputPayload {
                    run_id: run_id.clone(),
                    page_nodes: ia_build.page_nodes.clone(),
                    max_links_per_page: 3,
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "global_site_reconcile".to_string());
        let global_site_reconcile = ctx
            .start_activity(
                AlegriaActivities::run_global_site_reconcile_step,
                GlobalSiteReconcileInputPayload {
                    run_id: run_id.clone(),
                    scope: Some(scope.clone()),
                    page_nodes: ia_build.page_nodes.clone(),
                    link_recommendations: link_recommend.link_recommendations.clone(),
                    reconcile_reason: "seo_site_build_canonical_cutover@1".to_string(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "projection_barrier(global_site_reconcile)".to_string());
        let planning_projection_barrier = ctx
            .start_activity(
                AlegriaActivities::run_projection_barrier_audit_step,
                ProjectionBarrierAuditInputPayload {
                    run_id: run_id.clone(),
                    checkpoint: "projection_barrier(global_site_reconcile)".to_string(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        let page_nodes = if global_site_reconcile.page_nodes.is_empty() {
            ia_build.page_nodes.clone()
        } else {
            global_site_reconcile.page_nodes.clone()
        };
        let link_recommendations = if global_site_reconcile.link_recommendations.is_empty() {
            link_recommend.link_recommendations.clone()
        } else {
            global_site_reconcile.link_recommendations.clone()
        };
        if page_nodes.is_empty() {
            ctx.state_mut(|s| s.phase = "done:no_pages".to_string());
            return Ok(run_id);
        }

        let factual_fragments = support_bundle
            .iter()
            .map(|support| support.fragment_text.clone())
            .collect::<Vec<_>>();
        let page_total = page_nodes.len();
        let mut published_pages = 0usize;
        let page_keys = page_phase_keys(&plan);
        let mut publish_projection_barrier = None;

        for (page_index, page_node) in page_nodes.iter().cloned().enumerate() {
            let page_blueprint = ia_build
                .page_blueprints
                .iter()
                .find(|blueprint| blueprint.blueprint_key == page_node.blueprint_key)
                .cloned()
                .unwrap_or_default();
            let required_links: Vec<_> = link_recommendations
                .iter()
                .filter(|link| {
                    link.required_flag && link.source_page_key == page_node.page_node_key
                })
                .cloned()
                .collect();

            let mut draft_plan: Option<DraftAssembleOutputPayload> = None;
            let mut editorial_candidate: Option<EditorialDraftGenerateOutputPayload> = None;
            let mut draft: Option<DraftNormalizeOutputPayload> = None;
            let mut cms_requested: Option<CmsPublishOutputPayload> = None;
            let mut cms_approved: Option<CmsPublishOutputPayload> = None;
            let mut materialized: Option<PublishMaterializeOutputPayload> = None;
            let mut page_blocked = false;

            for key in page_keys.iter().copied() {
                ctx.state_mut(|s| s.phase = phase_with_ordinal(key, page_index, page_total));
                match key {
                    SeoPhaseKey::DraftAssemble => {
                        let _ = ctx
                            .start_activity(
                                AlegriaActivities::run_truth_admissibility_gate_step,
                                TruthAdmissibilityGateInput {
                                    run_id: run_id.clone(),
                                    context_key: site_input.context_key.clone(),
                                    applicant_profile: scope.applicant_profile.clone(),
                                    verified_support: support_bundle.clone(),
                                },
                                db_opts(30),
                            )
                            .await?;
                        draft_plan = Some(
                            ctx.start_activity(
                                AlegriaActivities::run_draft_assemble_step,
                                DraftAssembleInputPayload {
                                    run_id: run_id.clone(),
                                    page_node: Some(page_node.clone()),
                                    page_blueprint: Some(page_blueprint.clone()),
                                    factual_fragments: factual_fragments.clone(),
                                    verified_support: support_bundle.clone(),
                                    required_links: required_links.clone(),
                                    section_templates: Vec::new(),
                                    llm_candidate: None,
                                    source_context_chunks: Vec::new(),
                                },
                                db_opts(30),
                            )
                            .await?,
                        );
                    }
                    SeoPhaseKey::EditorialDraftGenerate => {
                        let draft_plan_ref =
                            draft_plan.as_ref().expect("draft_assemble required");
                        editorial_candidate = Some(
                            ctx.start_activity(
                                AlegriaActivities::run_editorial_draft_generate,
                                EditorialDraftGenerateInputPayload {
                                    run_id: run_id.clone(),
                                    request: draft_plan_ref.llm_request.clone(),
                                    provider_policy: "multi_provider:first_available".to_string(),
                                },
                                db_opts(60),
                            )
                            .await?,
                        );
                    }
                    SeoPhaseKey::DraftNormalize => {
                        let draft_plan_ref =
                            draft_plan.as_ref().expect("draft_assemble required");
                        let editorial_ref = editorial_candidate
                            .as_ref()
                            .expect("editorial generation required");
                        draft = Some(
                            ctx.start_activity(
                                AlegriaActivities::run_draft_normalize_step,
                                DraftNormalizeInputPayload {
                                    run_id: run_id.clone(),
                                    page_node: Some(page_node.clone()),
                                    page_blueprint: Some(page_blueprint.clone()),
                                    factual_fragments: factual_fragments.clone(),
                                    verified_support: support_bundle.clone(),
                                    required_links: required_links.clone(),
                                    section_templates: draft_plan_ref
                                        .editorial_brief
                                        .as_ref()
                                        .map(|brief| brief.section_templates.clone())
                                        .unwrap_or_default(),
                                    content_block_plan: draft_plan_ref.content_block_plan.clone(),
                                    llm_candidate: editorial_ref.candidate.clone(),
                                },
                                db_opts(30),
                            )
                            .await?,
                        );
                    }
                    SeoPhaseKey::ContentContractValidate => {
                        let draft_ref = draft.as_ref().expect("draft_normalize required");
                        let _ = ctx
                            .start_activity(
                                AlegriaActivities::run_content_contract_validate_step,
                                ContentContractValidateInputPayload {
                                    run_id: run_id.clone(),
                                    draft: draft_ref.draft.clone(),
                                    required_links: required_links.clone(),
                                },
                                db_opts(30),
                            )
                            .await?;
                    }
                    SeoPhaseKey::DraftQa => {
                        let draft_ref = draft.as_ref().expect("draft_normalize required");
                        let qa = ctx
                            .start_activity(
                                AlegriaActivities::run_draft_qa_step,
                                DraftQaInputPayload {
                                    run_id: run_id.clone(),
                                    draft: draft_ref.draft.clone(),
                                    supported_fragments: factual_fragments.clone(),
                                    required_links: required_links.clone(),
                                },
                                db_opts(30),
                            )
                            .await?;
                        if let Some(draft_state) = draft.as_mut().and_then(|state| state.draft.as_mut())
                        {
                            draft_state.qa_verdict = qa.verdict;
                        }
                    }
                    SeoPhaseKey::CmsRequestReview => {
                        let draft_ref = draft.as_ref().expect("draft_normalize required");
                        cms_requested = Some(
                            ctx.start_activity(
                                AlegriaActivities::run_cms_request_review_step,
                                CmsPublishInputPayload {
                                    run_id: run_id.clone(),
                                    page_node: Some(page_node.clone()),
                                    draft: draft_ref.draft.clone(),
                                    actor_role: "seo_system".to_string(),
                                    publish_mode: "request_review".to_string(),
                                    approval_decision: None,
                                },
                                db_opts(30),
                            )
                            .await?,
                        );
                        if cms_requested
                            .as_ref()
                            .expect("cms_request_review output")
                            .verdict
                            != "review_requested"
                        {
                            page_blocked = true;
                            ctx.state_mut(|s| {
                                s.phase = format!(
                                    "{}:{}",
                                    blocked_publish_gate_status(),
                                    phase_with_ordinal(key, page_index, page_total)
                                )
                            });
                            break;
                        }
                    }
                    SeoPhaseKey::HumanApprovalWait => {
                        let cms_requested_ref =
                            cms_requested.as_ref().expect("cms request required");
                        let approval_input = HumanApprovalWaitInput {
                            run_id: run_id.clone(),
                            page_node_key: page_node.page_node_key.clone(),
                            revision_id: cms_requested_ref.revision_id.clone(),
                        };
                        match plan.policy.interaction {
                            RunInteractionPolicy::AllowHitlPause => {
                                ctx.state_mut(|s| {
                                    s.waiting_hitl = true;
                                    s.resume_requested = false;
                                    s.paused = true;
                                    s.phase = phase_with_ordinal(key, page_index, page_total);
                                });
                                loop {
                                    ctx.wait_condition(|s| !s.paused && s.resume_requested)
                                        .await;
                                    let approval_decision: CmsApprovalDecision = ctx
                                        .start_activity(
                                            AlegriaActivities::run_human_approval_wait_step,
                                            approval_input.clone(),
                                            db_opts(30),
                                        )
                                        .await?;
                                    if approval_decision.decision == "approved" {
                                        break;
                                    }
                                    ctx.state_mut(|s| {
                                        s.waiting_hitl = true;
                                        s.resume_requested = false;
                                        s.paused = true;
                                        s.phase = format!(
                                            "{}:{}:{}",
                                            phase_label(key),
                                            approval_decision.decision,
                                            page_index + 1
                                        );
                                    });
                                }
                                ctx.state_mut(|s| {
                                    s.waiting_hitl = false;
                                    s.resume_requested = false;
                                });
                            }
                            RunInteractionPolicy::FailIfHitlRequired => {
                                page_blocked = true;
                                ctx.state_mut(|s| {
                                    s.waiting_hitl = false;
                                    s.resume_requested = false;
                                    s.phase = format!(
                                        "{}:{}",
                                        blocked_interaction_status(
                                            RunInteractionPolicy::FailIfHitlRequired
                                        ),
                                        phase_with_ordinal(key, page_index, page_total)
                                    );
                                });
                                break;
                            }
                            RunInteractionPolicy::RequirePreApprovedDecision => {
                                let approval_decision: CmsApprovalDecision = ctx
                                    .start_activity(
                                        AlegriaActivities::run_human_approval_wait_step,
                                        approval_input,
                                        db_opts(30),
                                    )
                                    .await?;
                                if approval_decision.decision != "approved" {
                                    page_blocked = true;
                                    ctx.state_mut(|s| {
                                        s.waiting_hitl = false;
                                        s.resume_requested = false;
                                        s.phase = format!(
                                            "{}:{}",
                                            blocked_interaction_status(
                                                RunInteractionPolicy::RequirePreApprovedDecision
                                            ),
                                            phase_with_ordinal(key, page_index, page_total)
                                        );
                                    });
                                    break;
                                }
                            }
                        }
                    }
                    SeoPhaseKey::CmsPublishApproved => {
                        let draft_ref = draft.as_ref().expect("draft_normalize required");
                        let cms_requested_ref =
                            cms_requested.as_ref().expect("cms request required");
                        let approval_lookup_key = format!(
                            "{}|{}|{}",
                            run_id, page_node.page_node_key, cms_requested_ref.revision_id
                        );
                        let approval_decision: CmsApprovalDecision = ctx
                            .start_activity(
                                AlegriaActivities::load_cms_approval_decision,
                                approval_lookup_key,
                                db_opts(30),
                            )
                            .await?;
                        cms_approved = Some(
                            ctx.start_activity(
                                AlegriaActivities::run_cms_publish_approved_step,
                                CmsPublishInputPayload {
                                    run_id: run_id.clone(),
                                    page_node: Some(page_node.clone()),
                                    draft: draft_ref.draft.clone(),
                                    actor_role: "seo_system".to_string(),
                                    publish_mode: "approved_publish".to_string(),
                                    approval_decision: Some(approval_decision),
                                },
                                db_opts(30),
                            )
                            .await?,
                        );
                        if cms_approved.as_ref().expect("cms approved output").verdict
                            != "approved"
                        {
                            page_blocked = true;
                            ctx.state_mut(|s| {
                                s.phase = format!(
                                    "{}:{}",
                                    blocked_publish_gate_status(),
                                    phase_with_ordinal(key, page_index, page_total)
                                )
                            });
                            break;
                        }
                    }
                    SeoPhaseKey::PublishMaterialize => {
                        let cms_requested_ref =
                            cms_requested.as_ref().expect("cms request required");
                        materialized = Some(
                            ctx.start_activity(
                                AlegriaActivities::run_publish_materialize_step,
                                PublishMaterializeInputPayload {
                                    run_id: run_id.clone(),
                                    page_node_key: page_node.page_node_key.clone(),
                                    revision_id: cms_requested_ref.revision_id.clone(),
                                    cms_document_id: cms_requested_ref.cms_document_id.clone(),
                                    canonical_url_path: page_node.canonical_url_path.clone(),
                                    publish_artifact: cms_requested_ref
                                        .publish_artifact
                                        .clone()
                                        .or(cms_approved.as_ref().and_then(|approved| {
                                            approved.publish_artifact.clone()
                                        })),
                                    output_dir: String::new(),
                                    base_url: String::new(),
                                },
                                db_opts(60),
                            )
                            .await?,
                        );
                        if materialized
                            .as_ref()
                            .expect("publish materialize output")
                            .materialization_status
                            .contains("failed")
                        {
                            page_blocked = true;
                            ctx.state_mut(|s| {
                                s.phase = format!(
                                    "{}:{}",
                                    blocked_publish_gate_status(),
                                    phase_with_ordinal(key, page_index, page_total)
                                )
                            });
                            break;
                        }
                    }
                    SeoPhaseKey::RenderPreviewValidate => {
                        let materialized_ref =
                            materialized.as_ref().expect("publish materialize required");
                        let render_validation = ctx
                            .start_activity(
                                AlegriaActivities::run_render_preview_validate_step,
                                RenderPreviewValidateInputPayload {
                                    run_id: run_id.clone(),
                                    preview_pages: materialized_ref.preview_pages.clone(),
                                },
                                db_opts(30),
                            )
                            .await?;
                        if render_validation.verdict != "render_ready" {
                            page_blocked = true;
                            ctx.state_mut(|s| {
                                s.phase = format!(
                                    "{}:{}",
                                    blocked_publish_gate_status(),
                                    phase_with_ordinal(key, page_index, page_total)
                                )
                            });
                            break;
                        }
                    }
                    SeoPhaseKey::FinalizePublish => {
                        let cms_requested_ref =
                            cms_requested.as_ref().expect("cms request required");
                        let materialized_ref =
                            materialized.as_ref().expect("publish materialize required");
                        let render_validation = ctx
                            .start_activity(
                                AlegriaActivities::run_render_preview_validate_step,
                                RenderPreviewValidateInputPayload {
                                    run_id: run_id.clone(),
                                    preview_pages: materialized_ref.preview_pages.clone(),
                                },
                                db_opts(30),
                            )
                            .await?;
                        let finalized = ctx
                            .start_activity(
                                AlegriaActivities::run_finalize_publish_step,
                                FinalizePublishInputPayload {
                                    run_id: run_id.clone(),
                                    page_node_key: page_node.page_node_key.clone(),
                                    revision_id: cms_requested_ref.revision_id.clone(),
                                    publish_artifact: materialized_ref.publish_artifact.clone(),
                                    render_validation: Some(render_validation),
                                },
                                db_opts(30),
                            )
                            .await?;
                        if finalized.verdict == "published" {
                            published_pages += 1;
                        } else {
                            page_blocked = true;
                            ctx.state_mut(|s| {
                                s.phase = format!(
                                    "{}:{}",
                                    blocked_publish_gate_status(),
                                    phase_with_ordinal(key, page_index, page_total)
                                )
                            });
                            break;
                        }
                    }
                    _ => {}
                }
                ctx.wait_condition(|s| !s.paused).await;
            }

            if page_blocked {
                continue;
            }
        }

        if page_keys.contains(&SeoPhaseKey::FinalizePublish) {
            ctx.state_mut(|s| s.phase = "projection_barrier(publish)".to_string());
            publish_projection_barrier = Some(
                ctx.start_activity(
                    AlegriaActivities::run_projection_barrier_audit_step,
                    ProjectionBarrierAuditInputPayload {
                        run_id: run_id.clone(),
                        checkpoint: "projection_barrier(publish)".to_string(),
                    },
                    db_opts(30),
                )
                .await?,
            );
            ctx.wait_condition(|s| !s.paused).await;
        }

        let mut rebuild = None;
        for key in final_phase_keys(&plan).iter().copied() {
            ctx.state_mut(|s| s.phase = phase_label(key).to_string());
            match key {
                SeoPhaseKey::RebuildDetect => {
                    rebuild = Some(
                        ctx.start_activity(
                            AlegriaActivities::run_rebuild_detect_step,
                            RebuildDetectInputPayload {
                                run_id: run_id.clone(),
                                changed_truth_keys: verified_truth_write
                                    .changed_truth_keys
                                    .clone(),
                                page_nodes: page_nodes.clone(),
                            },
                            db_opts(30),
                        )
                        .await?,
                    );
                }
                _ => {}
            }
            ctx.wait_condition(|s| !s.paused).await;
        }

        ctx.state_mut(|s| s.phase = "done:seo_site_build_canonical_cutover".to_string());
        metrics::global()
            .workflow_completions_total
            .with_label_values(&["SeoSiteBuildCanonicalCutoverWorkflow"])
            .inc();

        Ok(format!(
            "seo_site_build_canonical_cutover_ok run_id={} preflight_status={} support_bundle={} raw_pages={} semantic_pages={} page_utility_sections={} dom_blocked={} sections={} sectioning_blocked={} cas_blocked={} evidence_sections={} raw_evidence_barrier_status={} layer_router_hitl={} subspan_split={} entity_mentions={} canonical_mapping_hitl={} ontology_gate_hitl={} procedural_rules={} operational_entities={} editorial_topics={} seo_signals={} commercial_signals={} schema_invalid={} candidate_hitl={} triples={} completeness_hitl={} resolution_hitl={} contradiction_conflicts={} truth_verified={} truth_hitl={} verified_writes={} graph_gate_blocked={} retrieval_gate_blocked={} semantic_projection_barrier_blocked={} neo4j_failed={} qdrant_failed={} serp_patterns={} opportunity_clusters={} ia_pages={} link_recommendations={} global_reconcile_pages={} global_reconcile_links={} planning_projection_barrier_blocked={} published_pages={} publish_projection_barrier_blocked={} rebuild_verdict={}",
            run_id,
            preflight.status,
            support_bundle.len(),
            crawled.raw_page_ids.len(),
            semantic.page_count,
            page_utility.section_count,
            dom.blocked_section_count,
            sectioning.section_count,
            sectioning_contract.blocked_section_count,
            cas_gate.blocked_section_count,
            raw_evidence.section_count,
            raw_evidence_barrier.status,
            layer_router.needs_hitl_count,
            subspan_layer_router.needs_split_count,
            entity_spans.mention_count,
            canonical_mapping.needs_hitl_count,
            ontology_intake.needs_hitl_count,
            procedural.rule_count,
            operational.entity_count,
            editorial.topic_count,
            seo_signals.signal_count,
            commercial_signals.signal_count,
            schema_validate.invalid_section_count,
            candidate_validation.needs_hitl_count,
            triples.triple_count,
            completeness.needs_hitl_count,
            resolution.needs_hitl_count,
            contradiction.conflict_count,
            truth_adjudication.verified_count,
            truth_adjudication.needs_hitl_count,
            verified_truth_write.verified_rule_count,
            graph_gate.blocked_events,
            retrieval_gate.blocked_events,
            semantic_projection_barrier.blocked_events,
            neo4j.failed_events + neo4j.remaining_failed,
            qdrant.failed_events + qdrant.remaining_failed,
            serp_normalize.serp_patterns.len(),
            opportunity_build.keyword_clusters.len(),
            ia_build.page_nodes.len(),
            link_recommend.link_recommendations.len(),
            global_site_reconcile.page_nodes.len(),
            global_site_reconcile.link_recommendations.len(),
            planning_projection_barrier.blocked_events,
            published_pages,
            publish_projection_barrier
                .as_ref()
                .map(|barrier| barrier.blocked_events)
                .unwrap_or_default(),
            rebuild
                .as_ref()
                .map(|output| output.verdict.as_str())
                .unwrap_or("not_run")
        ))
    }

    #[signal(name = "pause")]
    fn pause(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.paused = true;
        self.phase = "paused".to_string();
    }

    #[signal(name = "resume")]
    fn resume(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        if self.waiting_hitl {
            self.resume_requested = true;
            self.paused = false;
            self.phase = "resumed_hitl_review".to_string();
        } else {
            self.paused = false;
            self.phase = "resumed".to_string();
        }
    }

    #[query(name = "status")]
    fn status(&self, _ctx: &WorkflowContextView) -> BasicWorkflowStatus {
        BasicWorkflowStatus {
            phase: self.phase.clone(),
            paused: self.paused,
        }
    }
}
