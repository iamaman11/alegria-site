use contracts::generated::alegria::temporal::v1::{
    CrawlSourcesInputPayload, ProjectionBarrierAuditInputPayload, SeoSiteBuildInputPayload,
    SeoVerifiedFactSupportState, SerpIngestInputPayload,
};
use infrastructure::adapters::temporalio_sdk_adapter::{
    workflow, workflow_methods, SyncWorkflowContext, WorkerOptions, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};
use seo_ports::VerifiedSupportBundleRequest;

use crate::activities::operations::{
    CanonicalMappingSweepInput, CasGateInput, DomBlockRelevanceSweepInput,
    EntitySpanSweepInput, LayerRouterSweepInput, OntologyIntakeGateInput,
    PageUtilitySweepInput, RawEvidenceRegisterInput, SectionSemanticGateBundle,
    SectioningContractGateInput, SectioningInput, SeoPreflightInput,
    SubspanLayerRouterInput, WholePageSemanticPassInput,
};
use crate::activities::AlegriaActivities;
use crate::metrics;

use super::runtime::{db_opts, BasicWorkflowStatus};

#[workflow]
#[derive(Default)]
struct SeoSiteBuildCanonicalCutoverWorkflow {
    paused: bool,
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
        let support_bundle: Vec<SeoVerifiedFactSupportState> = ctx
            .start_activity(
                AlegriaActivities::load_verified_support_bundle,
                support_bundle_request,
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
                    scope: Some(scope),
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

        ctx.state_mut(|s| s.phase = "done:seo_site_build_canonical_cutover".to_string());
        metrics::global()
            .workflow_completions_total
            .with_label_values(&["SeoSiteBuildCanonicalCutoverWorkflow"])
            .inc();

        Ok(format!(
            "seo_site_build_canonical_cutover_ok run_id={} preflight_status={} support_bundle={} raw_pages={} semantic_pages={} page_utility_sections={} dom_blocked={} sections={} sectioning_blocked={} cas_blocked={} evidence_sections={} raw_evidence_barrier_status={} layer_router_hitl={} subspan_split={} entity_mentions={} canonical_mapping_hitl={} ontology_gate_hitl={}",
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
            ontology_intake.needs_hitl_count
        ))
    }

    #[signal(name = "pause")]
    fn pause(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.paused = true;
        self.phase = "paused".to_string();
    }

    #[signal(name = "resume")]
    fn resume(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.paused = false;
        self.phase = "resumed".to_string();
    }

    #[query(name = "status")]
    fn status(&self, _ctx: &WorkflowContextView) -> BasicWorkflowStatus {
        BasicWorkflowStatus {
            phase: self.phase.clone(),
            paused: self.paused,
        }
    }
}
