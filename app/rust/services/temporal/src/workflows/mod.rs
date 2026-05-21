//! Workflows registry and factory for Temporal worker.

use infrastructure::adapters::temporalio_sdk_adapter::WorkerOptions;

// Required for proc-macro expansions.
use futures_util as _;
use infrastructure::adapters::temporalio_sdk_adapter::temporalio_common as _;

use crate::activities::AlegriaActivities;

mod content_generation;
mod expert_decomposed_extraction;
mod expert_extraction;
mod expert_projection;
mod expert_semantic_slice;
mod freshness;
mod projection_reconcile;
mod runtime;
mod seo_site_build;
mod test_hitl;

/// Собирает WorkerOptions со всеми зарегистрированными workflows и activities.
pub(crate) fn build_worker_options(task_queue: &str, acts: AlegriaActivities) -> WorkerOptions {
    let build_id = std::env::var("WORKER_BUILD_ID").unwrap_or_else(|_| "dev-local".to_string());
    tracing::info!(task_queue, build_id = %build_id, "building temporal worker options");
    let mut opts = WorkerOptions::new(task_queue)
        .register_activities(acts)
        .build();
    if std::env::var("ALLOW_LEGACY_CONTENT_WORKFLOW")
        .ok()
        .as_deref()
        == Some("true")
    {
        content_generation::register(&mut opts);
    }
    expert_decomposed_extraction::register(&mut opts);
    expert_extraction::register(&mut opts);
    expert_projection::register(&mut opts);
    expert_semantic_slice::register(&mut opts);
    freshness::register(&mut opts);
    projection_reconcile::register(&mut opts);
    seo_site_build::register(&mut opts);
    test_hitl::register(&mut opts);
    opts
}
