//! Opt-in native Temporal orchestrator for checked, provider-free DAGs.
//! Effectful manifest versions are preflight-only until independently
//! authorized owner execution has a durable per-instance admission bridge.
//!
//! This executable is a real package boundary, not another binary in the
//! provider-linked legacy `temporal_worker` Cargo package. Do not register
//! partial Activity sets on the legacy task queue or change V1/V2 histories.

use anyhow::{Context, Result};
use std::env;
use temporal_gateway::{connect_client, PURE_DAG_ORCHESTRATOR_QUEUE};
use temporalio_common::worker::{
    VersioningBehavior, WorkerDeploymentOptions, WorkerDeploymentVersion,
};
use temporalio_sdk::runtime::RuntimeOptions;
use temporalio_sdk::{Runtime, Worker, WorkerOptions};

// Required by native Temporal proc-macro expansions.
use futures_util as _;
use temporalio_common as _;

mod replay;
mod workflow;

const LIVE_OPT_IN: &str = "ALLOW_PURE_DAG_ORCHESTRATOR";

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveConfig {
    source_sha: String,
    temporal_url: String,
    namespace: String,
    versioning: bool,
}

fn immutable_source_sha(value: &str) -> bool {
    value.len() == 40
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && value.bytes().any(|byte| byte != b'0')
}

fn checked_live_config(read: impl Fn(&str) -> Option<String>) -> Result<LiveConfig> {
    anyhow::ensure!(
        read(LIVE_OPT_IN).as_deref() == Some("true"),
        "{LIVE_OPT_IN}=true is required; separate orchestrator is not a default deployment"
    );
    if let Some(queue) = read("TEMPORAL_ORCHESTRATOR_TASK_QUEUE") {
        anyhow::ensure!(
            queue == PURE_DAG_ORCHESTRATOR_QUEUE,
            "unknown orchestrator Task Queue: expected {PURE_DAG_ORCHESTRATOR_QUEUE}"
        );
    }
    let source_sha = read("WORKER_BUILD_ID").context(
        "WORKER_BUILD_ID must identify the exact committed 40-hex Git source for a new Workflow",
    )?;
    anyhow::ensure!(
        immutable_source_sha(&source_sha),
        "WORKER_BUILD_ID must be a nonzero 40-hex source SHA"
    );
    let temporal_url = read("TEMPORAL_URL").unwrap_or_else(|| "http://localhost:7233".to_string());
    let namespace = read("TEMPORAL_NAMESPACE").unwrap_or_else(|| "default".to_string());
    anyhow::ensure!(
        !namespace.trim().is_empty() && namespace.trim() == namespace,
        "TEMPORAL_NAMESPACE must be a nonempty exact name"
    );
    let versioning = match read("TEMPORAL_WORKER_VERSIONING").as_deref() {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => anyhow::bail!("TEMPORAL_WORKER_VERSIONING must be true or false"),
    };
    Ok(LiveConfig {
        source_sha,
        temporal_url,
        namespace,
        versioning,
    })
}

fn build_worker_options(config: &LiveConfig) -> Result<WorkerOptions> {
    let deployment_options = if config.versioning {
        Some(
            WorkerDeploymentOptions::new(
                WorkerDeploymentVersion::builder()
                    .deployment_name("alegria-pure-dag-v3".to_owned())
                    .build_id(config.source_sha.clone())
                    .build(),
            )
            .use_worker_versioning(true)
            .default_versioning_behavior(VersioningBehavior::Pinned)
            .build(),
        )
    } else {
        None
    };
    let mut options = WorkerOptions::new(PURE_DAG_ORCHESTRATOR_QUEUE)
        .maybe_deployment_options(deployment_options)
        .build();
    // Pure execution and effectful PRE-FLIGHT Workflows only. Both are
    // provider-free; zero Activities, child Workflows or persistence adapters
    // are registered. A prepared effect MUST fail non-retryably before I/O.
    workflow::register(&mut options)?;
    Ok(options)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Replay happens before opt-in config, network, runtime or subscriber.
    // A replay-only invocation must not require provider secrets or database.
    let mut args = env::args_os().skip(1);
    if let Some(argument) = args.next() {
        if argument == std::ffi::OsStr::new("--replay-history") {
            let path = args
                .next()
                .context("--replay-history requires one JSON file")?;
            anyhow::ensure!(args.next().is_none(), "unexpected replay arguments");
            replay::replay_history(std::path::Path::new(&path)).await?;
            println!("CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS");
            return Ok(());
        }
        anyhow::bail!("unexpected temporal_orchestrator argument: {argument:?}");
    }
    let config = checked_live_config(|name| env::var(name).ok())?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let options = RuntimeOptions::builder()
        .build()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let runtime = Runtime::from_current_tokio(options)?;
    let identity = format!(
        "alegria-pure-orchestrator@{}:{}:{}",
        env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".to_owned()),
        std::process::id(),
        config.source_sha
    );
    let client = connect_client(&config.temporal_url, identity, &config.namespace).await?;
    let worker_options = build_worker_options(&config)?;
    tracing::info!(
        task_queue = PURE_DAG_ORCHESTRATOR_QUEUE,
        source_sha = %config.source_sha,
        versioning = config.versioning,
        "starting isolated provider-free Temporal pure DAG worker"
    );
    Worker::new(&runtime, client, worker_options)
        .map_err(|error| anyhow::anyhow!("{error}"))?
        .run()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn valid_env() -> BTreeMap<String, String> {
        BTreeMap::from([
            (LIVE_OPT_IN.into(), "true".into()),
            (
                "WORKER_BUILD_ID".into(),
                "0123456789abcdef0123456789abcdef01234567".into(),
            ),
            (
                "TEMPORAL_ORCHESTRATOR_TASK_QUEUE".into(),
                PURE_DAG_ORCHESTRATOR_QUEUE.into(),
            ),
            ("TEMPORAL_URL".into(), "http://localhost:7233".into()),
        ])
    }

    #[test]
    fn live_start_requires_explicit_opt_in_exact_source_sha_and_distinct_queue() {
        let original = valid_env();
        let config = checked_live_config(|key| original.get(key).cloned()).unwrap();
        assert_eq!(config.namespace, "default");
        assert!(!config.versioning);
        let mut broken = original.clone();
        for (key, value) in [
            (LIVE_OPT_IN, "false"),
            ("WORKER_BUILD_ID", "dev-local"),
            ("TEMPORAL_ORCHESTRATOR_TASK_QUEUE", "alegria-pipeline"),
        ] {
            broken.insert(key.into(), value.into());
            assert!(
                checked_live_config(|name| broken.get(name).cloned()).is_err(),
                "{key}"
            );
            broken = original.clone();
        }
        broken.insert("TEMPORAL_NAMESPACE".into(), " ".into());
        assert!(checked_live_config(|name| broken.get(name).cloned()).is_err());
        broken = original.clone();
        broken.insert("TEMPORAL_WORKER_VERSIONING".into(), "1".into());
        assert!(checked_live_config(|name| broken.get(name).cloned()).is_err());
        broken = original;
        broken.remove(LIVE_OPT_IN);
        assert!(checked_live_config(|name| broken.get(name).cloned()).is_err());
    }

    #[test]
    fn provider_free_worker_registers_pure_workflow_without_any_activities() {
        let settings = valid_env();
        let config = checked_live_config(|key| settings.get(key).cloned()).unwrap();
        build_worker_options(&config).unwrap();
        let mut versioned = config;
        versioned.versioning = true;
        build_worker_options(&versioned).unwrap();
    }
}
