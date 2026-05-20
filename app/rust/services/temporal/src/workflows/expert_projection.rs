use contracts::generated::alegria::temporal::v1::{
    CrawlSourcesInputPayload, ProjectionBarrierAuditInputPayload,
    RawKnowledgeIngestionInputPayload, ReconcileTargetInputPayload, SeoSiteBuildInputPayload,
    SeoVerifiedFactSupportState, SerpIngestInputPayload,
};
use infrastructure::adapters::temporalio_sdk_adapter::{
    workflow, workflow_methods, SyncWorkflowContext, WorkerOptions, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};
use runtime_models::ReconcileTargetReportRecord;
use seo_ports::VerifiedSupportBundleRequest;

use crate::activities::AlegriaActivities;
use crate::metrics;

use super::runtime::{db_opts, BasicWorkflowStatus};

#[workflow]
#[derive(Default)]
struct ExpertProjectionWorkflow {
    paused: bool,
    phase: String,
}

pub(crate) fn register(opts: &mut WorkerOptions) {
    opts.register_workflow::<ExpertProjectionWorkflow>();
}

fn support_request(run_id: &str, site_input: &SeoSiteBuildInputPayload) -> WorkflowResult<String> {
    Ok(serde_json::to_string(&VerifiedSupportBundleRequest {
        run_id: run_id.to_string(),
        context_key: site_input.context_key.clone(),
        scope_signature: site_input
            .scope
            .as_ref()
            .map(|value| value.scope_signature.clone())
            .unwrap_or_default(),
        applicant_profile: site_input
            .scope
            .as_ref()
            .map(|value| value.applicant_profile.clone())
            .unwrap_or_default(),
    })
    .map_err(anyhow::Error::from)?)
}

async fn reconcile_target(
    ctx: &mut WorkflowContext<ExpertProjectionWorkflow>,
    run_id: &str,
    target_system: &str,
) -> WorkflowResult<ReconcileTargetReportRecord> {
    Ok(ctx
        .start_activity(
            AlegriaActivities::run_projection_reconcile_step,
            ReconcileTargetInputPayload {
                run_id: run_id.to_string(),
                target_system: target_system.to_string(),
                dry_run: false,
                max_retry_count: 10,
                batch_limit: 500,
                requeue_base_delay_sec: 0,
                requeue_jitter_sec: 15,
            },
            db_opts(120),
        )
        .await?)
}

#[workflow_methods]
impl ExpertProjectionWorkflow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        metrics::global()
            .workflow_starts_total
            .with_label_values(&["ExpertProjectionWorkflow"])
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
                    scope: site_input.scope.clone(),
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

        ctx.state_mut(|s| s.phase = "raw_knowledge_ingestion".to_string());
        let raw_knowledge = ctx
            .start_activity(
                AlegriaActivities::run_raw_knowledge_ingestion_step,
                RawKnowledgeIngestionInputPayload {
                    run_id: run_id.clone(),
                    context_key: site_input.context_key.clone(),
                    query_batch_key: ingest.query_batch_key.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                    source_policy: "candidate_only_truth_extraction@1".to_string(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

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

        ctx.state_mut(|s| s.phase = "neo4j_sync".to_string());
        let neo4j = reconcile_target(ctx, &run_id, "neo4j").await?;
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

        ctx.state_mut(|s| s.phase = "voyage_qdrant_sync".to_string());
        let qdrant = reconcile_target(ctx, &run_id, "qdrant").await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "projection_barrier.post_projection".to_string());
        let post_projection = ctx
            .start_activity(
                AlegriaActivities::run_projection_barrier_audit_step,
                ProjectionBarrierAuditInputPayload {
                    run_id: run_id.clone(),
                    checkpoint: "projection_barrier.post_projection".to_string(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        if !raw_knowledge.changed_truth_keys.is_empty() {
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

        ctx.state_mut(|s| s.phase = "done:expert_projection".to_string());
        metrics::global()
            .workflow_completions_total
            .with_label_values(&["ExpertProjectionWorkflow"])
            .inc();

        Ok(format!(
            "expert_projection_ok run_id={} verified_rules={} changed_truth_keys={} support_bundle={} graph_gate_blocked={} retrieval_gate_blocked={} post_projection_blocked={} neo4j_failed={} qdrant_failed={}",
            run_id,
            raw_knowledge.verified_rule_count,
            raw_knowledge.changed_truth_keys.len(),
            support_bundle.len(),
            graph_gate.blocked_events,
            retrieval_gate.blocked_events,
            post_projection.blocked_events,
            neo4j.failed_candidates,
            qdrant.failed_candidates
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
