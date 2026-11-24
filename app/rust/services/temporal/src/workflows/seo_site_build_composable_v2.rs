//! Opt-in, typed, history-backed V2 source acquisition executor.
//!
//! Only the explicitly supported `SourceAcquisitionOnly` functional bundles
//! may run here. Source acquisition is narrower than the existing SEO
//! CrawlIngestOnly scenario, which additionally requires raw knowledge ingest.
//! The canonical V1 site build remains the sole publication pipeline;
//! its historical sequence of commands must not be changed by this executor.

use std::time::Duration;

use contracts::generated::alegria::temporal::v1::{
    CrawlSourcesInputPayload, CrawlSourcesOutputPayload, SeoScopePayload,
    SeoSiteBuildInputPayload, SeoVerifiedFactSupportState, SerpIngestInputPayload,
};
use seo_application::execution::{SeoDurablePlan, SeoDurableStep, SEO_RUN_MODE_CRAWL_ONLY};
use seo_ports::VerifiedSupportBundleRequest;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{
    SyncWorkflowContext, WorkerOptions, WorkflowContext, WorkflowContextView, WorkflowResult,
};

use crate::activities::operations::SeoPreflightInput;
use crate::activities::AlegriaActivities;

use super::runtime::{db_opts, workflow_contract_failure, BasicWorkflowStatus};

#[workflow]
pub(super) struct SeoSiteBuildComposableV2Workflow {
    /// Deserialized once from the Temporal start event; never mutated.
    plan: SeoDurablePlan,
    paused: bool,
    phase: String,
}

pub(crate) fn register(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<SeoSiteBuildComposableV2Workflow>()?;
    Ok(())
}

#[workflow_methods]
impl SeoSiteBuildComposableV2Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, plan: SeoDurablePlan) -> Self {
        Self {
            plan,
            paused: false,
            phase: "validating_plan".to_string(),
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        // Preflight the entire plan, including schema version, duplicate
        // commands, dependency closure and scenario's required output, BEFORE
        // scheduling the very first side-effecting Activity.
        let immutable_plan = ctx.state(|state| state.plan.clone());
        immutable_plan
            .validate()
            .map_err(|error| workflow_contract_failure(error.to_string()))?;

        let run_id = ctx.info().workflow_id().to_string();
        ctx.state_mut(|state| state.phase = "load_seo_site_build_input".to_string());
        let site_input: SeoSiteBuildInputPayload = ctx
            .start_activity(
                AlegriaActivities::load_seo_site_build_input,
                run_id.clone(),
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|state| !state.paused).await?;
        // The V2 runner does not publish pages, gate human approvals, or
        // perform arbitrary site-building modes. Reject mismatched persisted
        // registrations rather than silently switching mode.
        if site_input.run_mode != SEO_RUN_MODE_CRAWL_ONLY {
            return Err(workflow_contract_failure(format!(
                "V2 SourceAcquisitionOnly requires persisted run_mode=crawl_only, got {}",
                site_input.run_mode
            )));
        }
        let scope = site_input.scope.clone().ok_or_else(|| {
            workflow_contract_failure("SeoSiteBuildInputPayload.scope is required")
        })?;

        ctx.state_mut(|state| state.phase = "seo_preflight".to_string());
        let _preflight = ctx
            .start_activity(
                AlegriaActivities::run_seo_preflight_step,
                SeoPreflightInput {
                    run_id: run_id.clone(),
                    context_key: site_input.context_key.clone(),
                    scope: scope.clone(),
                    raw_page_ids: Vec::new(),
                    projection_max_lag_ms: 300_000,
                    require_materialized_projections: false,
                    enforce_required_contract: true,
                    require_page_brief_projection: false,
                    gate_key: String::new(),
                    skip_provider_probes: false,
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|state| !state.paused).await?;

        let mut verified_support_count = 0_usize;
        let mut crawl_result: Option<CrawlSourcesOutputPayload> = None;
        for &step in immutable_plan.ordered_steps() {
            match step {
                SeoDurableStep::LoadVerifiedSupport => {
                    ctx.state_mut(|state| {
                        state.phase = "load_verified_support_bundle".to_string()
                    });
                    let support_request = serde_json::to_string(&VerifiedSupportBundleRequest {
                        run_id: run_id.clone(),
                        context_key: site_input.context_key.clone(),
                        scope_signature: scope.scope_signature.clone(),
                        applicant_profile: scope.applicant_profile.clone(),
                    })
                    .map_err(|error| workflow_contract_failure(error.to_string()))?;
                    let verified: Vec<SeoVerifiedFactSupportState> = ctx
                        .start_activity(
                            AlegriaActivities::load_verified_support_bundle,
                            support_request,
                            db_opts(30),
                        )
                        .await?;
                    verified_support_count = verified.len();
                    ctx.wait_condition(|state| !state.paused).await?;
                }
                SeoDurableStep::SourceAcquisition => {
                    crawl_result = Some(acquire_sources(ctx, &run_id, &site_input, &scope).await?);
                }
            }
        }

        // `validate` proves this result is present before any Activity runs.
        let crawled = crawl_result.ok_or_else(|| {
            workflow_contract_failure("V2 plan did not produce crawled evidence")
        })?;
        ctx.state_mut(|state| state.phase = "source_acquisition_complete".to_string());
        Ok(format!(
            "seo_source_acquisition_v2_ok run_id={run_id} source_status={} crawled={} verified_support={verified_support_count}",
            crawled.status, crawled.crawled_count,
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

/// Source acquisition keeps all Temporal Activity retries, continuation
/// sweeps, durable backoff timers and a distinct persisted identity per sweep.
/// The narrow V2 implementation is isolated so it cannot accidentally rewrite
/// V1's historical command stream on a worker deployment.
async fn acquire_sources(
    ctx: &mut WorkflowContext<SeoSiteBuildComposableV2Workflow>,
    run_id: &str,
    site_input: &SeoSiteBuildInputPayload,
    scope: &SeoScopePayload,
) -> WorkflowResult<CrawlSourcesOutputPayload> {
    ctx.state_mut(|state| state.phase = "serp_ingest".to_string());
    let ingest = ctx
        .start_activity(
            AlegriaActivities::run_serp_ingest_step,
            SerpIngestInputPayload {
                run_id: run_id.to_string(),
                query_batch_key: site_input.query_batch_key.clone(),
                scope: Some(scope.clone()),
                queries: site_input.queries.clone(),
            },
            db_opts(1800),
        )
        .await?;
    ctx.wait_condition(|state| !state.paused).await?;

    ctx.state_mut(|state| state.phase = "crawl_sources".to_string());
    let mut crawl_input = CrawlSourcesInputPayload {
        run_id: run_id.to_string(),
        query_batch_key: ingest.query_batch_key.clone(),
        limit: 5,
        emit_qdrant: true,
        sweep_index: 0,
    };
    let mut crawled: CrawlSourcesOutputPayload = ctx
        .start_activity(
            AlegriaActivities::run_crawl_sources_step,
            crawl_input.clone(),
            db_opts(600),
        )
        .await?;
    ctx.wait_condition(|state| !state.paused).await?;

    while crawled.status == "more" || crawled.status == "pending_retry" {
        if crawled.status == "pending_retry" {
            ctx.timer(Duration::from_secs(60)).await;
            ctx.wait_condition(|state| !state.paused).await?;
        }
        crawl_input.sweep_index = crawl_input.sweep_index.checked_add(1).ok_or_else(|| {
            workflow_contract_failure("V2 crawl sweep index exhausted")
        })?;
        let next = ctx
            .start_activity(
                AlegriaActivities::run_crawl_sources_step,
                crawl_input.clone(),
                db_opts(600),
            )
            .await?;
        if crawled.status == "more" && next.claimed_count == 0 {
            return Err(workflow_contract_failure("V2 crawl backlog is not claimable"));
        }
        crawled.claimed_count = crawled.claimed_count.saturating_add(next.claimed_count);
        crawled.crawled_count = crawled.crawled_count.saturating_add(next.crawled_count);
        crawled.failed_count = crawled.failed_count.saturating_add(next.failed_count);
        crawled.raw_page_count = crawled.raw_page_count.saturating_add(next.raw_page_count);
        crawled.raw_section_count = crawled
            .raw_section_count
            .saturating_add(next.raw_section_count);
        crawled.qdrant_event_count = crawled
            .qdrant_event_count
            .saturating_add(next.qdrant_event_count);
        crawled.raw_page_ids.extend(next.raw_page_ids);
        crawled.failed_urls.extend(next.failed_urls);
        crawled.status = next.status;
        ctx.wait_condition(|state| !state.paused).await?;
    }
    if crawled.status != "done" && crawled.status != "partial" {
        return Err(workflow_contract_failure(format!(
            "unexpected V2 crawl completion status: {}",
            crawled.status
        )));
    }
    if crawled.failed_count > 0 {
        crawled.status = "partial".to_string();
    }
    Ok(crawled)
}
