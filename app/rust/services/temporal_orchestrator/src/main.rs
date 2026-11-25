//! Opt-in native Temporal orchestrator for checked, provider-free DAGs.
//! The separate V4 opt-in can start owner child Workflows, but no owner
//! Activity can write before independently attested PG V4 operator admission.
//!
//! This executable is a real package boundary, not another binary in the
//! provider-linked legacy `temporal_worker` Cargo package. Do not register
//! partial Activity sets on the legacy task queue or change V1/V2 histories.

use anyhow::{Context, Result};
use operation_protocol::OPERATION_DAG_PARENT_QUEUE;
use qdrant_contracts::QDRANT_DAG_PARENT_QUEUE_V1;
use std::env;
use temporal_gateway::{
    connect_client, EFFECT_DAG_ORCHESTRATOR_V4_QUEUE, EFFECT_DAG_ORCHESTRATOR_V5_QUEUE,
    PURE_DAG_ORCHESTRATOR_QUEUE,
};
use temporalio_common::worker::{
    VersioningBehavior, WorkerDeploymentOptions, WorkerDeploymentVersion,
};
use temporalio_sdk::runtime::RuntimeOptions;
use temporalio_sdk::{Runtime, Worker, WorkerOptions};

// Required by native Temporal proc-macro expansions.
use futures_util as _;
use temporalio_common as _;

mod qdrant_child_wire_json;
mod owner_catalog;
mod replay;
mod workflow;

const LIVE_OPT_IN: &str = "ALLOW_PURE_DAG_ORCHESTRATOR";
const V4_LIVE_OPT_IN: &str = "ALLOW_EFFECT_DAG_ORCHESTRATOR_V4";
const V5_LIVE_OPT_IN: &str = "ALLOW_EFFECT_DAG_ORCHESTRATOR_V5";
const QDRANT_V1_LIVE_OPT_IN: &str = "ALLOW_QDRANT_DAG_ORCHESTRATOR_V1";
const OPERATION_V1_LIVE_OPT_IN: &str = "ALLOW_OPERATION_DAG_V1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrchestratorMode {
    PureV3,
    AdmittedEffectV4,
    AdmittedEffectV5,
    QdrantV1,
    OperationDagV1,
}

impl OrchestratorMode {
    fn task_queue(self) -> &'static str {
        match self {
            Self::PureV3 => PURE_DAG_ORCHESTRATOR_QUEUE,
            Self::AdmittedEffectV4 => EFFECT_DAG_ORCHESTRATOR_V4_QUEUE,
            Self::AdmittedEffectV5 => EFFECT_DAG_ORCHESTRATOR_V5_QUEUE,
            Self::QdrantV1 => QDRANT_DAG_PARENT_QUEUE_V1,
            Self::OperationDagV1 => OPERATION_DAG_PARENT_QUEUE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveConfig {
    mode: OrchestratorMode,
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
    let pure_opt_in = read(LIVE_OPT_IN);
    let effect_opt_in = read(V4_LIVE_OPT_IN);
    let v5_effect_opt_in = read(V5_LIVE_OPT_IN);
    let qdrant_opt_in = read(QDRANT_V1_LIVE_OPT_IN);
    let operation_v1_opt_in = read(OPERATION_V1_LIVE_OPT_IN);
    for (label, setting) in [
        (LIVE_OPT_IN, &pure_opt_in),
        (V4_LIVE_OPT_IN, &effect_opt_in),
        (V5_LIVE_OPT_IN, &v5_effect_opt_in),
        (QDRANT_V1_LIVE_OPT_IN, &qdrant_opt_in),
        (OPERATION_V1_LIVE_OPT_IN, &operation_v1_opt_in),
    ] {
        anyhow::ensure!(
            matches!(setting.as_deref(), None | Some("true") | Some("false")),
            "{label} must be exactly true or false"
        );
    }
    let mode = match (
        pure_opt_in.as_deref() == Some("true"),
        effect_opt_in.as_deref() == Some("true"),
        v5_effect_opt_in.as_deref() == Some("true"),
        qdrant_opt_in.as_deref() == Some("true"),
        operation_v1_opt_in.as_deref() == Some("true"),
    ) {
        (true, false, false, false, false) => OrchestratorMode::PureV3,
        (false, true, false, false, false) => OrchestratorMode::AdmittedEffectV4,
        (false, false, true, false, false) => OrchestratorMode::AdmittedEffectV5,
        (false, false, false, true, false) => OrchestratorMode::QdrantV1,
        (false, false, false, false, true) => OrchestratorMode::OperationDagV1,
        _ => anyhow::bail!("select exactly one of {LIVE_OPT_IN}, {V4_LIVE_OPT_IN}, {V5_LIVE_OPT_IN}, {QDRANT_V1_LIVE_OPT_IN}, {OPERATION_V1_LIVE_OPT_IN}=true"),
    };
    if let Some(queue) = read("TEMPORAL_ORCHESTRATOR_TASK_QUEUE") {
        anyhow::ensure!(
            queue == mode.task_queue(),
            "unknown orchestrator Task Queue: expected {}",
            mode.task_queue(),
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
        mode,
        source_sha,
        temporal_url,
        namespace,
        versioning,
    })
}

fn build_worker_options(config: &LiveConfig) -> Result<WorkerOptions> {
    let task_queue = config.mode.task_queue();
    let deployment_options = if config.versioning {
        Some(
            WorkerDeploymentOptions::new(
                WorkerDeploymentVersion::builder()
                    .deployment_name(task_queue.to_owned())
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
    let mut options = WorkerOptions::new(task_queue)
        .maybe_deployment_options(deployment_options)
        .build();
    // No Activity handlers on either queue. The V4 parent can only start
    // *typed child Workflows* on the independent PostgreSQL command queue.
    match config.mode {
        OrchestratorMode::PureV3 => workflow::register(&mut options)?,
        OrchestratorMode::AdmittedEffectV4 => workflow::register_v4(&mut options)?,
        OrchestratorMode::AdmittedEffectV5 => workflow::register_v5(&mut options)?,
        OrchestratorMode::QdrantV1 => workflow::register_qdrant_v1(&mut options)?,
        OrchestratorMode::OperationDagV1 => workflow::register_operation_dag_v1(&mut options)?,
    }
    Ok(options)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Replay happens before opt-in config, network, runtime or subscriber.
    // A replay-only invocation must not require provider secrets or database.
    let mut args = env::args_os().skip(1);
    if let Some(argument) = args.next() {
        if argument == std::ffi::OsStr::new("--replay-history")
            || argument == std::ffi::OsStr::new("--replay-effect-history")
            || argument == std::ffi::OsStr::new("--replay-v5-effect-history")
            || argument == std::ffi::OsStr::new("--replay-qdrant-effect-history")
            || argument == std::ffi::OsStr::new("--replay-operation-dag-v1-history")
        {
            let queue = if argument == std::ffi::OsStr::new("--replay-operation-dag-v1-history") {
                OPERATION_DAG_PARENT_QUEUE
            } else if argument == std::ffi::OsStr::new("--replay-qdrant-effect-history") {
                QDRANT_DAG_PARENT_QUEUE_V1
            } else if argument == std::ffi::OsStr::new("--replay-v5-effect-history") {
                EFFECT_DAG_ORCHESTRATOR_V5_QUEUE
            } else if argument == std::ffi::OsStr::new("--replay-effect-history") {
                EFFECT_DAG_ORCHESTRATOR_V4_QUEUE
            } else {
                PURE_DAG_ORCHESTRATOR_QUEUE
            };
            let path = args
                .next()
                .context("history replay requires one JSON file")?;
            anyhow::ensure!(args.next().is_none(), "unexpected replay arguments");
            replay::replay_history_for_queue(std::path::Path::new(&path), queue).await?;
            println!("CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS");
            return Ok(());
        }
        anyhow::bail!("unexpected temporal_orchestrator argument: {argument:?}");
    }
    let config = checked_live_config(|name| env::var(name).ok())?;
    if config.mode == OrchestratorMode::OperationDagV1 {
        // A caller-controlled DAG never chooses a Workflow Type or queue.
        // Only an operator-provisioned, process-local immutable catalog can
        // register owner contracts; a missing catalog is a startup failure.
        let path = env::var("TEMPORAL_OPERATION_DAG_CATALOG_PATH")
            .context("generic DAG requires an operator-owned trusted route catalog path")?;
        let catalog = owner_catalog::load_trusted_owner_catalog(std::path::Path::new(&path))?;
        workflow::install_owner_catalog(catalog)?;
    }
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
        "alegria-checked-orchestrator@{}:{}:{}",
        env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".to_owned()),
        std::process::id(),
        config.source_sha
    );
    let client = connect_client(&config.temporal_url, identity, &config.namespace).await?;
    let worker_options = build_worker_options(&config)?;
    tracing::info!(
        task_queue = config.mode.task_queue(),
        source_sha = %config.source_sha,
        versioning = config.versioning,
        "starting isolated provider-free Temporal operation orchestrator"
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

    #[test]
    fn v4_effect_parent_requires_distinct_queue_and_exclusive_opt_in() {
        let mut settings = valid_env();
        settings.insert(LIVE_OPT_IN.into(), "false".into());
        settings.insert(V4_LIVE_OPT_IN.into(), "true".into());
        settings.insert(
            "TEMPORAL_ORCHESTRATOR_TASK_QUEUE".into(),
            EFFECT_DAG_ORCHESTRATOR_V4_QUEUE.into(),
        );
        let config = checked_live_config(|key| settings.get(key).cloned()).unwrap();
        assert_eq!(config.mode, OrchestratorMode::AdmittedEffectV4);
        build_worker_options(&config).unwrap();
        let mut versioned = config;
        versioned.versioning = true;
        build_worker_options(&versioned).unwrap();

        settings.insert(LIVE_OPT_IN.into(), "true".into());
        assert!(checked_live_config(|key| settings.get(key).cloned()).is_err());
        settings.insert(LIVE_OPT_IN.into(), "false".into());
        settings.insert(
            "TEMPORAL_ORCHESTRATOR_TASK_QUEUE".into(),
            PURE_DAG_ORCHESTRATOR_QUEUE.into(),
        );
        assert!(checked_live_config(|key| settings.get(key).cloned()).is_err());
    }

    #[test]
    fn v5_ordered_parent_is_separately_gated_and_cannot_claim_v4_queue() {
        let mut settings = valid_env();
        settings.insert(LIVE_OPT_IN.into(), "false".into());
        settings.insert(V4_LIVE_OPT_IN.into(), "false".into());
        settings.insert(V5_LIVE_OPT_IN.into(), "true".into());
        settings.insert(
            "TEMPORAL_ORCHESTRATOR_TASK_QUEUE".into(),
            EFFECT_DAG_ORCHESTRATOR_V5_QUEUE.into(),
        );
        let cfg = checked_live_config(|name| settings.get(name).cloned()).unwrap();
        assert_eq!(cfg.mode, OrchestratorMode::AdmittedEffectV5);
        build_worker_options(&cfg).unwrap();
        let mut versioned = cfg;
        versioned.versioning = true;
        build_worker_options(&versioned).unwrap();

        settings.insert(V4_LIVE_OPT_IN.into(), "true".into());
        assert!(checked_live_config(|name| settings.get(name).cloned()).is_err());
        settings.insert(V4_LIVE_OPT_IN.into(), "false".into());
        settings.insert(
            "TEMPORAL_ORCHESTRATOR_TASK_QUEUE".into(),
            EFFECT_DAG_ORCHESTRATOR_V4_QUEUE.into(),
        );
        assert!(checked_live_config(|name| settings.get(name).cloned()).is_err());
        settings.insert(V5_LIVE_OPT_IN.into(), "1".into());
        assert!(checked_live_config(|name| settings.get(name).cloned()).is_err());
    }

    #[test]
    fn generic_operation_parent_has_its_own_strictly_opt_in_queue() {
        let mut settings = valid_env();
        settings.insert(LIVE_OPT_IN.into(), "false".into());
        settings.insert(OPERATION_V1_LIVE_OPT_IN.into(), "true".into());
        settings.insert(
            "TEMPORAL_ORCHESTRATOR_TASK_QUEUE".into(),
            OPERATION_DAG_PARENT_QUEUE.into(),
        );
        let cfg = checked_live_config(|name| settings.get(name).cloned()).unwrap();
        assert_eq!(cfg.mode, OrchestratorMode::OperationDagV1);
        build_worker_options(&cfg).unwrap();
        let mut pinned = cfg;
        pinned.versioning = true;
        build_worker_options(&pinned).unwrap();

        settings.insert(QDRANT_V1_LIVE_OPT_IN.into(), "true".into());
        assert!(checked_live_config(|name| settings.get(name).cloned()).is_err());
        settings.insert(QDRANT_V1_LIVE_OPT_IN.into(), "false".into());
        settings.insert(
            "TEMPORAL_ORCHESTRATOR_TASK_QUEUE".into(),
            QDRANT_DAG_PARENT_QUEUE_V1.into(),
        );
        assert!(checked_live_config(|name| settings.get(name).cloned()).is_err());
    }
}
