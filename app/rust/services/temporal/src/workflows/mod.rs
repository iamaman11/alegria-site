//! Workflows registry and factory for Temporal worker.

use temporalio_common::worker::{
    VersioningBehavior, WorkerDeploymentOptions, WorkerDeploymentVersion,
};
use temporalio_sdk::WorkerOptions;

// Required for proc-macro expansions.
use futures_util as _;
use temporalio_common as _;

use crate::activities::AlegriaActivities;

mod expert_decomposed_extraction;
mod expert_extraction;
mod expert_projection;
mod expert_semantic_slice;
mod freshness;
mod projection_reconcile;
mod runtime;
mod seo_site_build_canonical_cutover;
mod test_hitl;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerProfile {
    Production,
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

/// A pinned deployment must refer to a concrete source revision, not a
/// human-chosen label that can silently be reused for incompatible workflow
/// command histories. Local workers with versioning disabled keep their
/// existing flexible development Build IDs.
fn immutable_source_build_id(build_id: &str) -> bool {
    let value = build_id.trim();
    value.len() == 40
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && value.bytes().any(|byte| byte != b'0')
}

fn pinned_deployment_options(
    enabled: bool,
    build_id: &str,
    deployment_name: &str,
) -> anyhow::Result<Option<WorkerDeploymentOptions>> {
    if !enabled {
        return Ok(None);
    }
    if !immutable_source_build_id(build_id) {
        anyhow::bail!(
            "TEMPORAL_WORKER_VERSIONING requires WORKER_BUILD_ID to be an immutable 40-hex source commit SHA"
        );
    }
    if deployment_name.trim().is_empty() {
        anyhow::bail!("TEMPORAL_DEPLOYMENT_NAME must not be empty");
    }
    Ok(Some(
        WorkerDeploymentOptions::new(
            WorkerDeploymentVersion::builder()
                .deployment_name(deployment_name.to_owned())
                .build_id(build_id.to_owned())
                .build(),
        )
        .use_worker_versioning(true)
        .default_versioning_behavior(VersioningBehavior::Pinned)
        .build(),
    ))
}

#[cfg(test)]
mod deployment_version_tests {
    use super::{immutable_source_build_id, pinned_deployment_options};

    #[test]
    fn native_pin_requires_real_immutable_build_identity() {
        let source_sha = "0123456789abcdef0123456789abcdef01234567";
        for build_id in [
            "",
            "docker-dev",
            "dev-local",
            " ",
            "f00dead",
            "0123456789abcdef0123456789abcdef0123456",
            "0123456789abcdef0123456789abcdef012345678",
            "0123456789abcdef0123456789abcdef0123456g",
            "0000000000000000000000000000000000000000",
        ] {
            assert!(!immutable_source_build_id(build_id));
            assert!(pinned_deployment_options(true, build_id, "alegria-site").is_err());
        }
        assert!(immutable_source_build_id(source_sha));
        assert!(pinned_deployment_options(true, source_sha, "").is_err());
        assert!(pinned_deployment_options(true, source_sha, "alegria-site")
            .unwrap()
            .is_some());
    }

    #[test]
    fn unversioned_legacy_workers_remain_explicitly_opt_out() {
        assert!(pinned_deployment_options(false, "docker-dev", "")
            .unwrap()
            .is_none());
    }
}

/// Собирает WorkerOptions со всеми зарегистрированными workflows и activities.
pub(crate) fn build_worker_options(
    task_queue: &str,
    acts: AlegriaActivities,
) -> anyhow::Result<WorkerOptions> {
    let build_id = std::env::var("WORKER_BUILD_ID").unwrap_or_else(|_| "dev-local".to_string());
    let profile = WorkerProfile::from_env();
    tracing::info!(task_queue, build_id = %build_id, worker_profile = ?profile, "building temporal worker options");
    // Native Temporal pinning prevents new workflow histories being replayed
    // by a different build. Legacy unversioned histories require a drain.
    let deployment_name =
        std::env::var("TEMPORAL_DEPLOYMENT_NAME").unwrap_or_else(|_| "alegria-site".to_string());
    let deployment_options = pinned_deployment_options(
        env_flag("TEMPORAL_WORKER_VERSIONING"),
        &build_id,
        &deployment_name,
    )?;
    let mut opts = WorkerOptions::new(task_queue)
        .maybe_deployment_options(deployment_options)
        .register_activities(acts)
        .build();
    if env_flag("ALLOW_EXPERT_MIGRATION_WORKFLOWS") || profile.allows_all() {
        expert_decomposed_extraction::register(&mut opts)?;
        expert_extraction::register(&mut opts)?;
        expert_projection::register(&mut opts)?;
        expert_semantic_slice::register(&mut opts)?;
    }
    freshness::register(&mut opts)?;
    projection_reconcile::register(&mut opts)?;
    seo_site_build_canonical_cutover::register(&mut opts)?;
    if env_flag("ALLOW_TEST_HITL_WORKFLOW") || profile.allows_test() {
        test_hitl::register(&mut opts)?;
    }
    Ok(opts)
}
