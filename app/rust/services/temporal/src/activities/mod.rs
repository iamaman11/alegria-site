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

include!("registry/mod_registry_impl.rs");
