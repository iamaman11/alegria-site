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
mod seo_site_build_canonical_cutover;
mod test_hitl;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerProfile {
    Production,
    Compat,
    Test,
    All,
}

impl WorkerProfile {
    fn from_env() -> Self {
        match std::env::var("TEMPORAL_WORKER_PROFILE")
            .unwrap_or_else(|_| "production".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "compat" => Self::Compat,
            "test" => Self::Test,
            "all" => Self::All,
            "production" | "" => Self::Production,
            other => {
                tracing::warn!(
                    profile = other,
                    "unknown TEMPORAL_WORKER_PROFILE; using production profile"
                );
                Self::Production
            }
        }
    }

    fn allows_compat(self) -> bool {
        matches!(self, Self::Compat | Self::All)
    }

    fn allows_test(self) -> bool {
        matches!(self, Self::Test | Self::All)
    }

    fn allows_all(self) -> bool {
        matches!(self, Self::All)
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).ok().as_deref() == Some("true")
}

/// Собирает WorkerOptions со всеми зарегистрированными workflows и activities.
pub(crate) fn build_worker_options(task_queue: &str, acts: AlegriaActivities) -> WorkerOptions {
    let build_id = std::env::var("WORKER_BUILD_ID").unwrap_or_else(|_| "dev-local".to_string());
    let profile = WorkerProfile::from_env();
    tracing::info!(task_queue, build_id = %build_id, worker_profile = ?profile, "building temporal worker options");
    let mut opts = WorkerOptions::new(task_queue)
        .register_activities(acts)
        .build();
    if env_flag("ALLOW_LEGACY_CONTENT_WORKFLOW") || profile.allows_all() {
        content_generation::register(&mut opts);
    }
    if env_flag("ALLOW_EXPERT_MIGRATION_WORKFLOWS") || profile.allows_all() {
        expert_decomposed_extraction::register(&mut opts);
        expert_extraction::register(&mut opts);
        expert_projection::register(&mut opts);
        expert_semantic_slice::register(&mut opts);
    }
    freshness::register(&mut opts);
    projection_reconcile::register(&mut opts);
    seo_site_build_canonical_cutover::register(&mut opts);
    if env_flag("ALLOW_COMPAT_SEO_SITE_BUILD_WORKFLOW") || profile.allows_compat() {
        seo_site_build::register(&mut opts);
    }
    if env_flag("ALLOW_TEST_HITL_WORKFLOW") || profile.allows_test() {
        test_hitl::register(&mut opts);
    }
    opts
}
